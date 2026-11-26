describe("ES2024 RegExp /v (unicodeSets)", () => {
  it("supports unicode property escapes and subtraction/intersection/union inside classes", () => {
    const spacing = /^[\p{Spacing_Mark}--[\u1734\u302E\u302F]]+$/v;
    expect(spacing.unicode).toBe(false);
    expect(spacing.unicodeSets).toBe(true);
    expect(spacing.test("\u0903")).toBe(true);
    expect(spacing.test("\u1734")).toBe(false);
    expect(spacing.test("\u302E")).toBe(false);

    const asciiDigit = /^[\p{ASCII}&&\p{Decimal_Number}]+$/v;
    expect(asciiDigit.test("0123456789")).toBe(true);
    expect(asciiDigit.test("a")).toBe(false);
    expect(asciiDigit.test("\u0660")).toBe(false); // Arabic-Indic digit

    const nestedUnion = /^[[a-c][x-z]]+$/v;
    expect(nestedUnion.test("abxz")).toBe(true);
    expect(nestedUnion.test("m")).toBe(false);

    const chainedSub = /^[[a-f]--b--d]$/v;
    expect(chainedSub.test("a")).toBe(true);
    expect(chainedSub.test("b")).toBe(false);
    expect(chainedSub.test("d")).toBe(false);
    expect(chainedSub.test("e")).toBe(true);

    const chainedInter = /^[\p{Letter}&&\p{ASCII}&&\p{Lowercase}]$/v;
    expect(chainedInter.test("z")).toBe(true);
    expect(chainedInter.test("Z")).toBe(false);
    expect(chainedInter.test("é")).toBe(false);
  });

  it("supports \\q{} string disjunctions and negated classes", () => {
    const re = /^[\q{foo|bar|c|}[x]]$/v;
    expect(re.test("foo")).toBe(true);
    expect(re.test("bar")).toBe(true);
    expect(re.test("c")).toBe(true);
    expect(re.test("")).toBe(true);
    expect(re.test("x")).toBe(true);
    expect(re.test("baz")).toBe(false);

    // Intersection with non-string set removes mayContainStrings, allowing [^...]
    expect(/^[^\p{ASCII}&&\p{RGI_Emoji}]$/v.test("a")).toBe(true);
    expect(/^[\q{ab|cd}--\q{ab}]$/v.test("ab")).toBe(false);
    expect(/^[\q{ab|cd}--\q{ab}]$/v.test("cd")).toBe(true);
  });

  it("supports Unicode 17.0 properties of strings (RGI_Emoji and subsets)", () => {
    const rgi = /^\p{RGI_Emoji}$/v;
    expect(rgi.test("😀")).toBe(true);
    expect(rgi.test("⚠️")).toBe(true);
    expect(rgi.test("1️⃣")).toBe(true);
    expect(rgi.test("👍🏽")).toBe(true);
    expect(rgi.test("🇺🇸")).toBe(true);
    expect(rgi.test("🏴󠁧󠁢󠁥󠁮󠁧󠁿")).toBe(true);
    expect(rgi.test("👨‍👩‍👧")).toBe(true);
    expect(rgi.test("👨🏻‍🐰‍👨🏼")).toBe(true); // Unicode 17.0
    expect(rgi.test("A")).toBe(false);

    expect(/^\p{Basic_Emoji}$/v.test("⚠️")).toBe(true);
    expect(/^\p{Emoji_Keycap_Sequence}$/v.test("1️⃣")).toBe(true);
    expect(/^\p{RGI_Emoji_Modifier_Sequence}$/v.test("👍🏽")).toBe(true);
    expect(/^\p{RGI_Emoji_Flag_Sequence}$/v.test("🇨🇶")).toBe(true);
    expect(/^\p{RGI_Emoji_Tag_Sequence}$/v.test("🏴󠁧󠁢󠁥󠁮󠁧󠁿")).toBe(true);
    expect(/^\p{RGI_Emoji_ZWJ_Sequence}$/v.test("👨‍👩‍👧")).toBe(true);

    // Longest match + backtracking + lookbehind
    expect(/^\p{RGI_Emoji}\u200d👩‍👧$/v.test("👨‍👩‍👧")).toBe(true);
    expect(/(?<=\p{RGI_Emoji})x/v.test("👨‍👩‍👧x")).toBe(true);
    expect(/^[\p{RGI_Emoji}--\q{👨‍👩‍👧}]$/v.test("👨‍👩‍👧")).toBe(false);
    expect(/^[\p{RGI_Emoji}--\q{👨‍👩‍👧}]$/v.test("😀")).toBe(true);
  });

  it("handles /iv case folding and /vg surrogate advancement", () => {
    expect(/^[\p{Lowercase}--a]$/iv.test("A")).toBe(false);
    expect(/^[\p{Lowercase}--a]$/iv.test("B")).toBe(true);
    expect(/^[[^a]&&[a]]$/iv.test("a")).toBe(false);
    expect([..."😀a😀".matchAll(/^(?:)|(?=.)/vg)].length).toBeGreaterThan(2);
    expect("😀".match(/$/vg)?.length).toBe(1);
    expect("😀".replace(/\p{RGI_Emoji}/vg, "X")).toBe("X");
  });

  it("enforces /v class syntax errors", () => {
    for (const bad of [
      "[-a]",
      "[a-]",
      "[a--]",
      "[a&&]",
      "[ab--c]",
      "[a--bc]",
      "[a-z&&b-y]",
      "[a&&b-y]",
      "[^\\q{ab}]",
      "[^\\p{RGI_Emoji}]",
      "[!!]",
      "[&&]",
      "[(]",
      "[\\q{a}|[b]]",
    ]) {
      expect(() => new RegExp(bad, "v")).toThrow(SyntaxError);
    }
    expect(() => new RegExp(".*", "uv")).toThrow(SyntaxError);
    expect(() => new RegExp("\\P{RGI_Emoji}", "v")).toThrow(SyntaxError);
    expect(() => new RegExp("\\p{RGI_Emoji}", "u")).toThrow(SyntaxError);
  });
});
