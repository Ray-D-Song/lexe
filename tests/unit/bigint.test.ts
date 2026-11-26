describe("BigInt size limit", () => {
  it("supports BigInt values above 1 Mi-bit (e.g. 1n << BigInt(1 << 20))", () => {
    const bound = 1n << BigInt(1 << 20);
    const neg = -bound;
    expect(bound > 0n).toBe(true);
    expect(neg < 0n).toBe(true);
    expect(bound - 1n < bound).toBe(true);
    expect(bound >> BigInt(1 << 20)).toBe(1n);
    expect((-bound) >> BigInt(1 << 20)).toBe(-1n);
  });

  it("throws RangeError when exceeding the 16 Mi-bit maximum allocation limit", () => {
    expect(() => 1n << BigInt(1 << 25)).toThrow(RangeError);
  });
});
