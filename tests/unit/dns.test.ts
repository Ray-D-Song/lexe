import type dns from "node:dns";

import defaultImport from "node:dns";
import legacyImport from "dns";

it("node:dns should be the same as dns", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

const { lookup, promises } = defaultImport;

// Promise wrapper for dns.lookup
const dnsLookupAsync = (
  hostname: string,
  options?: number | dns.LookupOptions
) =>
  new Promise<dns.LookupAddress>((resolve, reject) => {
    lookup(hostname, options as any, (err, address, family) => {
      if (err) reject(err);
      else resolve({ address, family });
    });
  });

describe("lookup", () => {
  it("localhost name resolution should be possible (optionless)", async () => {
    const { address, family } = await dnsLookupAsync("localhost");
    expect(address === "::1" || address === "127.0.0.1").toBeTruthy();
    expect(family === 4 || family === 6).toBeTruthy();
  });

  it("localhost name resolution should be possible (integer option)", async () => {
    const { address, family } = await dnsLookupAsync("localhost", 4);
    expect(address).toEqual("127.0.0.1");
    expect(family).toEqual(4);
  });

  it("localhost name resolution should be possible (record option)", async () => {
    const { address, family } = await dnsLookupAsync("localhost", {
      family: 4,
    });
    expect(address).toEqual("127.0.0.1");
    expect(family).toEqual(4);
  });

  if (process.platform !== "linux") {
    it("Name resolution for localhost2 should result in an error (integer option)", async () => {
      await expect(dnsLookupAsync("localhost2", 4)).rejects.toThrow("known");
    });

    it("Name resolution for localhost2 should result in an error (optionless)", async () => {
      await expect(dnsLookupAsync("localhost2")).rejects.toThrow("known");
    });

    it("Name resolution for localhost2 should result in an error (record option)", async () => {
      await expect(dnsLookupAsync("localhost2", { family: 4 })).rejects.toThrow(
        "known"
      );
    });
  }
});

describe("dns/promises", () => {
  it("should load dns/promises and node:dns/promises", async () => {
    const a = await import("dns/promises");
    const b = await import("node:dns/promises");
    expect(typeof a.lookup).toBe("function");
    expect(a.lookup).toBe(a.default.lookup);
    expect(b.lookup).toBe(a.lookup);
  });

  it("default and named lookup should be the same function", async () => {
    const mod = await import("dns/promises");
    expect(mod.lookup).toBe(mod.default.lookup);
  });

  it('lookup("localhost") should return a single address object', async () => {
    const { lookup: lookupPromise } = await import("dns/promises");
    const result = await lookupPromise("localhost");
    expect(typeof result.address).toBe("string");
    expect(result.family === 4 || result.family === 6).toBeTruthy();
  });

  it("should return IPv4 for family: 4", async () => {
    const { lookup: lookupPromise } = await import("dns/promises");
    const result = await lookupPromise("localhost", { family: 4 });
    expect(result.address).toBe("127.0.0.1");
    expect(result.family).toBe(4);
  });

  it("should return a non-empty array for all: true", async () => {
    const { lookup: lookupPromise } = await import("dns/promises");
    const result = await lookupPromise("localhost", { all: true });
    expect(Array.isArray(result)).toBeTruthy();
    expect(result.length > 0).toBeTruthy();
  });

  if (process.platform !== "linux") {
    it("should reject for a non-existent host", async () => {
      const { lookup: lookupPromise } = await import("dns/promises");
      await expect(lookupPromise("localhost2")).rejects.toThrow("known");
    });
  }

  it("dns.promises.lookup should be the same as dns/promises.lookup", async () => {
    const mod = await import("dns/promises");
    expect(promises.lookup).toBe(mod.lookup);
  });

  it("should not leak internal DNS cache keys onto globalThis", () => {
    expect((globalThis as any).__lexeDnsPromisesLookup).toBeUndefined();
    expect(Object.keys(globalThis).includes("__lexeDnsPromisesLookup")).toBeFalsy();
  });
});
