describe("navigator", () => {
  it("exposes userAgent, hardwareConcurrency, language, languages, and platform", () => {
    expect(navigator.userAgent.startsWith("lexe ")).toBeTruthy();
    expect(typeof navigator.hardwareConcurrency).toBe("number");
    expect(navigator.hardwareConcurrency).toBeGreaterThanOrEqual(1);
    expect(Number.isInteger(navigator.hardwareConcurrency)).toBe(true);

    expect(typeof navigator.language).toBe("string");
    expect(navigator.language.length).toBeGreaterThan(0);

    expect(Array.isArray(navigator.languages)).toBe(true);
    expect(Object.isFrozen(navigator.languages)).toBe(true);
    expect(navigator.languages.length).toBeGreaterThanOrEqual(1);
    expect(navigator.languages[0]).toBe(navigator.language);

    expect(typeof navigator.platform).toBe("string");
    expect(navigator.platform.length).toBeGreaterThan(0);
    expect((navigator as any)[Symbol.toStringTag]).toBe("Navigator");
  });
});
