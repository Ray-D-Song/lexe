import net from "node:net";

// Node parity for the `net` address helpers added for backlog item 13:
// `isIPv4`/`isIPv6`, `BlockList`, `SocketAddress` and the auto-select-family
// getters/setters. Expectations were taken from Node 24.21 rather than from the
// documentation — the coercion and family rules below are not obvious.

describe("net.isIPv4 / net.isIPv6", () => {
  it("classifies boundary IPv4 literals", () => {
    expect(net.isIPv4("0.0.0.0")).toBe(true);
    expect(net.isIPv4("255.255.255.255")).toBe(true);
    expect(net.isIPv4("127.0.0.1")).toBe(true);
    // Out of range, too few/many octets, leading zero, hex form.
    expect(net.isIPv4("256.1.1.1")).toBe(false);
    expect(net.isIPv4("1.2.3")).toBe(false);
    expect(net.isIPv4("1.2.3.4.5")).toBe(false);
    expect(net.isIPv4("01.2.3.4")).toBe(false);
    expect(net.isIPv4("0x7f.1")).toBe(false);
  });

  it("classifies IPv6 literals including zone ids", () => {
    expect(net.isIPv6("::1")).toBe(true);
    expect(net.isIPv6("::")).toBe(true);
    expect(net.isIPv6("fe80::1%eth0")).toBe(true);
    expect(net.isIPv6("::1%lo")).toBe(true);
    expect(net.isIPv6("::ffff:1.2.3.4")).toBe(true);
    expect(net.isIPv6("::ffff:256.1.1.1")).toBe(false);
    expect(net.isIPv6("fe80::1%a/b")).toBe(false);
  });

  it("does not accept whitespace, empty, or IPv4 with a zone id", () => {
    expect(net.isIPv4("")).toBe(false);
    expect(net.isIPv4("1.2.3.4 ")).toBe(false);
    expect(net.isIPv4(" 1.2.3.4")).toBe(false);
    expect(net.isIPv4("1.2.3.4%x")).toBe(false);
    expect(net.isIPv6("1.2.3.4%x")).toBe(false);
  });

  it("returns false rather than throwing for non-string input", () => {
    for (const value of [null, undefined, 4, true, {}, []]) {
      expect(net.isIPv4(value as any)).toBe(false);
      expect(net.isIPv6(value as any)).toBe(false);
    }
  });

  it("stringifies String wrappers and Buffers like Node", () => {
    // Node checks `typeof === 'string' || isArrayBufferView` and then String()s.
    expect(net.isIPv4(new String("1.2.3.4") as any)).toBe(true);
    expect(net.isIPv4(Buffer.from("1.2.3.4") as any)).toBe(true);
  });
});

describe("net.BlockList", () => {
  it("matches addresses and subnets", () => {
    const list = new net.BlockList();
    list.addAddress("1.2.3.4");
    list.addSubnet("10.0.0.0", 8, "ipv4");
    list.addSubnet("2001:db8::", 32, "ipv6");
    list.addRange("5.0.0.1", "5.0.0.10", "ipv4");

    expect(list.check("1.2.3.4")).toBe(true);
    expect(list.check("1.2.3.5")).toBe(false);
    expect(list.check("10.1.2.3")).toBe(true);
    expect(list.check("11.0.0.1")).toBe(false);
    expect(list.check("5.0.0.5")).toBe(true);
    expect(list.check("5.0.0.11")).toBe(false);
    expect(list.check("2001:db8::1", "ipv6")).toBe(true);
    expect(list.check("::2", "ipv6")).toBe(false);
  });

  it("defaults to the IPv4 table when no family is given", () => {
    const list = new net.BlockList();
    list.addAddress("::1", "ipv6");
    // Without an explicit family an IPv6 literal never matches.
    expect(list.check("::1")).toBe(false);
    expect(list.check("::1", "ipv6")).toBe(true);
  });

  it("checks IPv4-mapped IPv6 literals against the IPv4 table", () => {
    const list = new net.BlockList();
    list.addSubnet("10.0.0.0", 8, "ipv4");
    expect(list.check("::ffff:10.1.2.3", "ipv6")).toBe(true);
    // ...but not when no family is given (that only consults the v4 table
    // with a v4 parse, and the literal is not a v4 address).
    expect(list.check("::ffff:10.1.2.3")).toBe(false);
  });

  it("treats an unrecognised family string as the IPv6 table", () => {
    const v4 = new net.BlockList();
    v4.addAddress("1.2.3.4");
    const v6 = new net.BlockList();
    v6.addAddress("::1", "ipv6");
    // Node does not reject unknown strings here; it routes them to IPv6.
    expect(v4.check("1.2.3.4", "bogus")).toBe(false);
    expect(v6.check("::1", "bogus")).toBe(true);
    expect(v4.check("1.2.3.4", "")).toBe(false);
    expect(v4.check("1.2.3.4", "IPV4")).toBe(true);
  });

  it("throws ERR_INVALID_ARG_TYPE for a non-string family on check", () => {
    const list = new net.BlockList();
    for (const family of [null, 4, 4.5]) {
      let code = "";
      try {
        list.check("1.2.3.4", family as any);
      } catch (e: any) {
        code = e.code;
      }
      expect(code).toBe("ERR_INVALID_ARG_TYPE");
    }
  });

  it("dedupes addresses but not subnets", () => {
    const addresses = new net.BlockList();
    addresses.addAddress("1.2.3.4");
    addresses.addAddress("1.2.3.4");
    expect(addresses.rules.length).toBe(1);

    const subnets = new net.BlockList();
    subnets.addSubnet("10.0.0.0", 8, "ipv4");
    subnets.addSubnet("10.0.0.0", 8, "ipv4");
    expect(subnets.rules.length).toBe(2);

    // Overlapping rules of different shape are both kept.
    const overlapping = new net.BlockList();
    overlapping.addAddress("1.2.3.4");
    overlapping.addSubnet("1.2.3.0", 24, "ipv4");
    expect(overlapping.rules.length).toBe(2);
    expect(overlapping.check("1.2.3.4")).toBe(true);
    expect(overlapping.check("1.2.3.9")).toBe(true);
  });

  it("requires an explicit ipv6 family for IPv6 rules", () => {
    for (const add of [
      (l: net.BlockList) => l.addAddress("::1"),
      (l: net.BlockList) => l.addSubnet("2001:db8::", 32),
      (l: net.BlockList) => l.addAddress("::ffff:1.2.3.4"),
    ]) {
      const list = new net.BlockList();
      let code = "";
      try {
        add(list);
      } catch (e: any) {
        code = e.code;
      }
      expect(code).toBe("ERR_INVALID_ADDRESS");
    }

    // The mapped literal is IPv6 here: `ipv6` accepts it, `ipv4` rejects it.
    const list = new net.BlockList();
    list.addAddress("::ffff:1.2.3.4", "ipv6");
    expect(list.rules.length).toBe(1);
    expect(list.check("::ffff:1.2.3.4", "ipv6")).toBe(true);
    expect(() => new net.BlockList().addAddress("::ffff:1.2.3.4", "ipv4")).toThrow();
  });

  it("rejects an out-of-range prefix with ERR_OUT_OF_RANGE", () => {
    let code = "";
    try {
      new net.BlockList().addSubnet("10.0.0.0", 33, "ipv4");
    } catch (e: any) {
      code = e.code;
    }
    expect(code).toBe("ERR_OUT_OF_RANGE");
  });

  it("rejects a reversed range", () => {
    let code = "";
    try {
      new net.BlockList().addRange("5.0.0.9", "5.0.0.1", "ipv4");
    } catch (e: any) {
      code = e.code;
    }
    expect(code).toBe("ERR_INVALID_ARG_VALUE");
  });

  it("rejects an unknown family in add* with ERR_INVALID_ARG_VALUE", () => {
    let code = "";
    try {
      new net.BlockList().addAddress("1.2.3.4", "bogus");
    } catch (e: any) {
      code = e.code;
    }
    expect(code).toBe("ERR_INVALID_ARG_VALUE");
  });
});

describe("net.SocketAddress", () => {
  it("defaults to 127.0.0.1 (ipv4)", () => {
    const address = new net.SocketAddress();
    expect(address.address).toBe("127.0.0.1");
    expect(address.family).toBe("ipv4");
    expect(address.port).toBe(0);
    expect(address.flowlabel).toBe(0);
  });

  it("accepts IPv4 and serialises with toJSON", () => {
    const address = new net.SocketAddress({ address: "1.2.3.4", port: 80 });
    expect(address.address).toBe("1.2.3.4");
    expect(address.family).toBe("ipv4");
    expect(address.port).toBe(80);
    expect(address.toJSON()).toEqual({
      address: "1.2.3.4",
      port: 80,
      family: "ipv4",
      flowlabel: 0,
    });
  });

  it("requires an explicit ipv6 family for an IPv6 address", () => {
    expect(() => new net.SocketAddress({ address: "::1" })).toThrow();
    const address = new net.SocketAddress({
      address: "::1",
      family: "ipv6",
      port: 443,
      flowlabel: 7,
    });
    expect(address.address).toBe("::1");
    expect(address.family).toBe("ipv6");
    expect(address.flowlabel).toBe(7);
  });

  it("rejects an address that does not match the declared family", () => {
    expect(() => new net.SocketAddress({ address: "1.2.3.4", family: "ipv6" })).toThrow();
    expect(() => new net.SocketAddress({ address: "nope" })).toThrow();
  });
});

describe("net auto-select-family defaults", () => {
  it("round-trips the family flag", () => {
    const original = net.getDefaultAutoSelectFamily();
    try {
      net.setDefaultAutoSelectFamily(false);
      expect(net.getDefaultAutoSelectFamily()).toBe(false);
      net.setDefaultAutoSelectFamily(true);
      expect(net.getDefaultAutoSelectFamily()).toBe(true);
    } finally {
      net.setDefaultAutoSelectFamily(original);
    }
  });

  it("round-trips the attempt timeout and bounds it", () => {
    const original = net.getDefaultAutoSelectFamilyAttemptTimeout();
    try {
      net.setDefaultAutoSelectFamilyAttemptTimeout(500);
      expect(net.getDefaultAutoSelectFamilyAttemptTimeout()).toBe(500);
      for (const bad of [0, -1]) {
        let code = "";
        try {
          net.setDefaultAutoSelectFamilyAttemptTimeout(bad);
        } catch (e: any) {
          code = e.code;
        }
        expect(code).toBe("ERR_OUT_OF_RANGE");
      }
    } finally {
      net.setDefaultAutoSelectFamilyAttemptTimeout(original);
    }
  });

  it("rejects a non-boolean family flag", () => {
    expect(() => net.setDefaultAutoSelectFamily("x" as any)).toThrow();
  });
});
