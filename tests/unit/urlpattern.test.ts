import { URLPattern as ModuleURLPattern } from "node:url";

describe("URLPattern", () => {
  it("matches global and module exports", () => {
    expect(ModuleURLPattern).toBe(globalThis.URLPattern);
    expect(URLPattern.name).toBe("URLPattern");
    expect(URLPattern.length).toBe(0);
    expect(URLPattern.prototype.exec.length).toBe(0);
    expect(URLPattern.prototype.test.length).toBe(0);
  });

  it("constructs default wildcard pattern", () => {
    const p = new URLPattern();
    expect(p.protocol).toBe("*");
    expect(p.username).toBe("*");
    expect(p.password).toBe("*");
    expect(p.hostname).toBe("*");
    expect(p.port).toBe("*");
    expect(p.pathname).toBe("*");
    expect(p.search).toBe("*");
    expect(p.hash).toBe("*");
    expect(p.hasRegExpGroups).toBe(false);
    expect(p.test()).toBe(true);
    expect(p.test({})).toBe(true);
  });

  it("parses constructor strings with groups, wildcards, and baseURL", () => {
    const p1 = new URLPattern("https://example.com/books/:id\\?sort=:sort#sec-:sec");
    expect(p1.protocol).toBe("https");
    expect(p1.hostname).toBe("example.com");
    expect(p1.port).toBe("");
    expect(p1.pathname).toBe("/books/:id");
    expect(p1.search).toBe("sort=:sort");
    expect(p1.hash).toBe("sec-:sec");
    expect(p1.hasRegExpGroups).toBe(false);

    const res = p1.exec("https://example.com/books/42?sort=asc#sec-intro");
    expect(res).toEqual({
      inputs: ["https://example.com/books/42?sort=asc#sec-intro"],
      protocol: { input: "https", groups: {} },
      username: { input: "", groups: { 0: "" } },
      password: { input: "", groups: { 0: "" } },
      hostname: { input: "example.com", groups: {} },
      port: { input: "", groups: {} },
      pathname: { input: "/books/42", groups: { id: "42" } },
      search: { input: "sort=asc", groups: { sort: "asc" } },
      hash: { input: "sec-intro", groups: { sec: "intro" } },
    });

    const pRel = new URLPattern("files/*", "https://example.com/base/dir/page.html");
    expect(pRel.pathname).toBe("/base/dir/files/*");
    expect(pRel.exec("/base/dir/files/a/b.txt", "https://example.com")?.pathname.groups).toEqual({
      0: "a/b.txt",
    });
  });

  it("supports custom regex groups, modifiers (?, *, +), and ignoreCase", () => {
    const p = new URLPattern({
      protocol: "http{s}?",
      hostname: "{:subdomain.}?example.com",
      pathname: "/users/:id(\\d+)/:rest*",
    });
    expect(p.hasRegExpGroups).toBe(true);

    const hit1 = p.exec("https://api.example.com/users/99/profile/settings");
    expect(hit1?.protocol.groups).toEqual({});
    expect(hit1?.hostname.groups).toEqual({ subdomain: "api" });
    expect(hit1?.pathname.groups).toEqual({ id: "99", rest: "profile/settings" });

    const hit2 = p.exec("http://example.com/users/7");
    expect(hit2).not.toBeNull();
    expect(Object.prototype.hasOwnProperty.call(hit2!.hostname.groups, "subdomain")).toBe(true);
    expect(hit2!.hostname.groups.subdomain).toBeUndefined();
    expect(hit2!.pathname.groups.id).toBe("7");
    expect(Object.prototype.hasOwnProperty.call(hit2!.pathname.groups, "rest")).toBe(true);
    expect(hit2!.pathname.groups.rest).toBeUndefined();

    expect(p.test("https://example.com/users/abc")).toBe(false);

    const opt = new URLPattern("https://example.com/books/:id?");
    const optMiss = opt.exec("https://example.com/books");
    expect(optMiss).not.toBeNull();
    expect(Object.prototype.hasOwnProperty.call(optMiss!.pathname.groups, "id")).toBe(true);
    expect(optMiss!.pathname.groups.id).toBeUndefined();

    const plus = new URLPattern({ pathname: "/files/:path+" });
    expect(plus.test({ pathname: "/files" })).toBe(false);
    expect(plus.exec({ pathname: "/files/a/b" })?.pathname.groups).toEqual({ path: "a/b" });

    const ic = new URLPattern(
      { username: "Alice", pathname: "/API/:name", search: "A=:val", hash: "Sec" },
      { ignoreCase: true }
    );
    expect(ic.test({ username: "Alice", pathname: "/api/Bob", search: "a=1", hash: "sec" })).toBe(true);
    expect(ic.test({ username: "alice", pathname: "/api/Bob", search: "a=1", hash: "sec" })).toBe(false);
  });

  it("canonicalizes components, baseURL inheritance, IPv6, and opaque URLs", () => {
    const unicode = new URLPattern("https://münchen.de/:p");
    expect(unicode.hostname).toBe("xn--mnchen-3ya.de");

    const ports = new URLPattern("wss://example.com:443/ws");
    expect(ports.port).toBe("");

    const ipv6 = new URLPattern("http://\\[\\:\\:1\\]:8080/a");
    expect(ipv6.hostname).toBe("[\\:\\:1]");
    expect(ipv6.test("http://[::1]:8080/a")).toBe(true);

    const inherited = new URLPattern({
      search: "?a=1",
      baseURL: "https://example.com:8080/dir?b=2#h",
    });
    expect(inherited.protocol).toBe("https");
    expect(inherited.hostname).toBe("example.com");
    expect(inherited.port).toBe("8080");
    expect(inherited.pathname).toBe("/dir");
    expect(inherited.search).toBe("a=1");
    expect(inherited.hash).toBe("*");

    const opaque = new URLPattern("data\\:text/plain,:x");
    expect(opaque.protocol).toBe("data");
    expect(opaque.hostname).toBe("");
    expect(opaque.pathname).toBe("text/plain,:x");
  });

  it("validates invalid constructor and exec inputs", () => {
    expect(() => new URLPattern("/relative-without-base")).toThrow(TypeError);
    expect(() => new URLPattern({ pathname: "/a" }, "https://example.com" as any)).toThrow(TypeError);
    expect(() => new URLPattern({ pathname: "/:id/:id" })).toThrow(TypeError);
    expect(() => new URLPattern({ pathname: "/(a(b))" })).toThrow(TypeError);
    expect(() => new URLPattern({ pathname: "/()" })).toThrow(TypeError);
    expect(() => new URLPattern({ pathname: "/(?:a)" })).toThrow(TypeError);
    expect(() => new URLPattern({ port: "65536" })).toThrow(TypeError);

    const p = new URLPattern("https://example.com/*");
    expect(p.exec("not-a-valid-url")).toBeNull();
    expect(() => p.exec({ pathname: "/a" }, "https://example.com")).toThrow(TypeError);
  });
});
