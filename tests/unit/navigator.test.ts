describe("navigator.userAgent", () => {
  it('should start with "lexe "', () => {
    expect(navigator.userAgent.startsWith("lexe ")).toBeTruthy();
  });
});
