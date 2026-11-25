import dns from "node:dns";
import dnsp from "node:dns/promises";
import dgram from "node:dgram";
import net from "node:net";

const D: any = dns;
const DP: any = dnsp;

function encodeDomainName(name: string): Buffer {
  const clean = name.endsWith(".") ? name.slice(0, -1) : name;
  if (!clean) return Buffer.from([0]);
  const parts: Buffer[] = [];
  for (const label of clean.split(".")) {
    const b = Buffer.from(label, "utf8");
    parts.push(Buffer.from([b.length]), b);
  }
  parts.push(Buffer.from([0]));
  return Buffer.concat(parts);
}

function readQnameAndEnd(buf: Buffer, offset = 12): { qname: string; qtype: number; end: number } {
  const labels: string[] = [];
  let pos = offset;
  while (pos < buf.length && buf[pos] !== 0) {
    const len = buf[pos];
    pos += 1;
    labels.push(buf.subarray(pos, pos + len).toString("utf8"));
    pos += len;
  }
  pos += 1; // null terminator
  const qtype = buf.readUInt16BE(pos);
  return { qname: labels.join("."), qtype, end: pos + 4 };
}

function buildDnsResponse(
  req: Buffer,
  rcode: number,
  tc: boolean,
  answers: Array<{ type: number; ttl?: number; rdata: Buffer }>
): Buffer {
  const { end } = readQnameAndEnd(req, 12);
  const hdr = Buffer.alloc(12);
  req.copy(hdr, 0, 0, 2); // ID
  hdr[2] = 0x81 | (tc ? 0x02 : 0x00); // QR=1, RD=1, TC
  hdr[3] = 0x80 | (rcode & 0x0f); // RA=1, RCODE
  hdr.writeUInt16BE(1, 4); // QDCOUNT
  hdr.writeUInt16BE(answers.length, 6); // ANCOUNT

  const question = req.subarray(12, end);
  const chunks: Buffer[] = [hdr, question];
  for (const ans of answers) {
    const rr = Buffer.alloc(12);
    rr.writeUInt16BE(0xc00c, 0); // Compression pointer to question QNAME
    rr.writeUInt16BE(ans.type, 2);
    rr.writeUInt16BE(1, 4); // IN
    rr.writeUInt32BE(ans.ttl ?? 60, 6);
    rr.writeUInt16BE(ans.rdata.length, 10);
    chunks.push(rr, ans.rdata);
  }
  return Buffer.concat(chunks);
}

const cbCall = <T = any>(fn: (cb: (err: any, ...rest: any[]) => void) => void): Promise<T> =>
  new Promise((resolve, reject) => {
    fn((err: any, ...rest: any[]) => (err ? reject(err) : resolve(rest as any as T)));
  });

describe("dns and dns/promises Node.js parity", () => {
  it("exports full Node.js v24 constants, methods, and Resolver classes without globalThis leaks", () => {
    const expectedDnsKeys = [
      "ADDRCONFIG",
      "ADDRGETNETWORKPARAMS",
      "ALL",
      "BADFAMILY",
      "BADFLAGS",
      "BADHINTS",
      "BADNAME",
      "BADQUERY",
      "BADRESP",
      "BADSTR",
      "CANCELLED",
      "CONNREFUSED",
      "DESTRUCTION",
      "EOF",
      "FILE",
      "FORMERR",
      "NODATA",
      "NOMEM",
      "NONAME",
      "NOTFOUND",
      "NOTIMP",
      "NOTINITIALIZED",
      "REFUSED",
      "Resolver",
      "SERVFAIL",
      "TIMEOUT",
      "V4MAPPED",
      "getDefaultResultOrder",
      "getServers",
      "lookup",
      "lookupService",
      "promises",
      "resolve",
      "resolve4",
      "resolve6",
      "resolveAny",
      "resolveCaa",
      "resolveCname",
      "resolveMx",
      "resolveNaptr",
      "resolveNs",
      "resolvePtr",
      "resolveSoa",
      "resolveSrv",
      "resolveTlsa",
      "resolveTxt",
      "reverse",
      "setDefaultResultOrder",
      "setServers",
    ];
    for (const k of expectedDnsKeys) {
      expect(D[k]).not.toBeUndefined();
    }

    for (const k of expectedDnsKeys) {
      if (k === "ADDRCONFIG" || k === "V4MAPPED" || k === "ALL" || k === "promises") {
        continue;
      }
      expect(DP[k]).not.toBeUndefined();
      expect(D.promises[k]).toBe(DP[k]);
    }

    expect(typeof D.ADDRCONFIG).toBe("number");
    expect(typeof D.V4MAPPED).toBe("number");
    expect(typeof D.ALL).toBe("number");
    expect(D.NOTFOUND).toBe("ENOTFOUND");
    expect(D.NODATA).toBe("ENODATA");
    expect(D.TIMEOUT).toBe("ETIMEOUT");
    expect(D.CANCELLED).toBe("ECANCELLED");
    expect((globalThis as any).__lexeDnsPromisesLookup).toBeUndefined();
  });

  it("validates and formats getServers / setServers cleanly", () => {
    const resolver = new D.Resolver();
    const initial = resolver.getServers();
    expect(Array.isArray(initial)).toBe(true);
    expect(initial.length).toBeGreaterThan(0);

    resolver.setServers(["127.0.0.1", "127.0.0.1:5353", "[::1]:5353", "::1"]);
    expect(resolver.getServers()).toEqual([
      "127.0.0.1",
      "127.0.0.1:5353",
      "[::1]:5353",
      "::1",
    ]);
    // Per-instance servers do not mutate global default servers
    expect(D.getServers()).toEqual(initial);

    for (const bad of ["not-an-ip", "127.0.0.1:0", "127.0.0.1:99999", "[::1"]) {
      let err: any;
      try {
        resolver.setServers([bad]);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(TypeError);
      expect(err?.code).toBe("ERR_INVALID_IP_ADDRESS");
    }

    // Invalid local address also throws ERR_INVALID_IP_ADDRESS
    expect(() => resolver.setLocalAddress("127.0.0.1")).not.toThrow();
    let localErr: any;
    try {
      resolver.setLocalAddress("bad-ip");
    } catch (e) {
      localErr = e;
    }
    expect(localErr?.code).toBe("ERR_INVALID_IP_ADDRESS");
  });

  it("supports getDefaultResultOrder / setDefaultResultOrder and lookup hints", async () => {
    const prevOrder = D.getDefaultResultOrder();
    try {
      expect(["verbatim", "ipv4first", "ipv6first"]).toContain(prevOrder);
      D.setDefaultResultOrder("ipv4first");
      expect(D.getDefaultResultOrder()).toBe("ipv4first");
      expect(DP.getDefaultResultOrder()).toBe("ipv4first");

      const allLocal = await DP.lookup("localhost", { all: true });
      const firstV6 = allLocal.findIndex((a: any) => a.family === 6);
      const lastV4 = allLocal.findLastIndex
        ? allLocal.findLastIndex((a: any) => a.family === 4)
        : allLocal.map((a: any) => a.family).lastIndexOf(4);
      if (firstV6 !== -1 && lastV4 !== -1) {
        expect(lastV4).toBeLessThan(firstV6);
      }

      D.setDefaultResultOrder("ipv6first");
      const allV6First = await DP.lookup("localhost", { all: true });
      const firstV4 = allV6First.findIndex((a: any) => a.family === 4);
      const lastV6 = allV6First.map((a: any) => a.family).lastIndexOf(6);
      if (firstV4 !== -1 && lastV6 !== -1) {
        expect(lastV6).toBeLessThan(firstV4);
      }

      let badOrderErr: any;
      try {
        D.setDefaultResultOrder("invalid-order");
      } catch (e) {
        badOrderErr = e;
      }
      expect(badOrderErr).toBeInstanceOf(TypeError);
      expect(badOrderErr?.code).toBe("ERR_INVALID_ARG_VALUE");

      // Lookup with hints constants
      const hinted = await DP.lookup("localhost", { hints: D.ADDRCONFIG | D.V4MAPPED });
      expect(typeof hinted.address).toBe("string");
      expect([4, 6]).toContain(hinted.family);
    } finally {
      D.setDefaultResultOrder(prevOrder);
    }
  });

  it("supports lookupService callback and promise forms plus validation errors", async () => {
    const [hostCb, svcCb] = await cbCall<any>((cb) => D.lookupService("127.0.0.1", 80, cb));
    expect(typeof hostCb).toBe("string");
    expect(svcCb === "http" || svcCb === "80").toBe(true);

    const svcObj = await DP.lookupService("127.0.0.1", 22);
    expect(typeof svcObj.hostname).toBe("string");
    expect(svcObj.service === "ssh" || svcObj.service === "22").toBe(true);

    let badIpErr: any;
    try {
      D.lookupService("bad-ip", 80, () => {});
    } catch (e) {
      badIpErr = e;
    }
    expect(badIpErr).toBeInstanceOf(TypeError);
    expect(badIpErr?.code).toBe("ERR_INVALID_ARG_VALUE");

    let badPortErr: any;
    try {
      D.lookupService("127.0.0.1", 99999, () => {});
    } catch (e) {
      badPortErr = e;
    }
    expect(badPortErr).toBeInstanceOf(RangeError);
    expect(badPortErr?.code).toBe("ERR_SOCKET_BAD_PORT");
  });

  it("resolves every DNS record type and reverse PTR over local mock DNS server", async () => {
    const srv = dgram.createSocket("udp4");
    const lastQueries: Array<{ qname: string; qtype: number }> = [];

    srv.on("message", (msg, rinfo) => {
      const { qname, qtype } = readQnameAndEnd(msg, 12);
      lastQueries.push({ qname, qtype });

      if (qtype === 1) {
        const resp = buildDnsResponse(msg, 0, false, [
          { type: 1, ttl: 120, rdata: Buffer.from([10, 20, 30, 40]) },
        ]);
        srv.send(resp, rinfo.port, rinfo.address);
      } else if (qtype === 28) {
        const ipv6 = Buffer.from("20010db8000000000000000000000001", "hex");
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 28, ttl: 90, rdata: ipv6 }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 5) {
        // CNAME using compression pointer back to question QNAME + prefix label "alias."
        const rdata = Buffer.concat([Buffer.from([5]), Buffer.from("alias"), Buffer.from([0xc0, 0x0c])]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 5, rdata }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 2) {
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 2, rdata: encodeDomainName("ns1.mock.test") }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 12) {
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 12, rdata: encodeDomainName("host.mock.test") }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 15) {
        const mx = Buffer.concat([Buffer.from([0, 10]), encodeDomainName("mail.mock.test")]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 15, rdata: mx }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 16) {
        const txt = Buffer.concat([
          Buffer.from([3]),
          Buffer.from("foo"),
          Buffer.from([3]),
          Buffer.from("bar"),
        ]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 16, rdata: txt }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 33) {
        const hdr = Buffer.alloc(6);
        hdr.writeUInt16BE(1, 0); // priority
        hdr.writeUInt16BE(5, 2); // weight
        hdr.writeUInt16BE(8443, 4); // port
        const rdata = Buffer.concat([hdr, encodeDomainName("srv.mock.test")]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 33, rdata }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 6) {
        const tail = Buffer.alloc(20);
        tail.writeUInt32BE(20261125, 0);
        tail.writeInt32BE(3600, 4);
        tail.writeInt32BE(600, 8);
        tail.writeInt32BE(86400, 12);
        tail.writeUInt32BE(300, 16);
        const rdata = Buffer.concat([
          encodeDomainName("ns1.mock.test"),
          encodeDomainName("admin.mock.test"),
          tail,
        ]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 6, rdata }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 35) {
        const hdr = Buffer.alloc(4);
        hdr.writeUInt16BE(100, 0);
        hdr.writeUInt16BE(10, 2);
        const f = Buffer.from("u");
        const s = Buffer.from("E2U+sip");
        const r = Buffer.from("!^.*$!sip:x@mock.test!");
        const rdata = Buffer.concat([
          hdr,
          Buffer.from([f.length]),
          f,
          Buffer.from([s.length]),
          s,
          Buffer.from([r.length]),
          r,
          encodeDomainName("next.mock.test"),
        ]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 35, rdata }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 257) {
        const rdata = Buffer.concat([Buffer.from([128, 5]), Buffer.from("issue"), Buffer.from("ca.example")]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 257, rdata }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 52) {
        const rdata = Buffer.from([3, 1, 1, 0xaa, 0xbb, 0xcc, 0xdd]);
        srv.send(
          buildDnsResponse(msg, 0, false, [{ type: 52, rdata }]),
          rinfo.port,
          rinfo.address
        );
      } else if (qtype === 255) {
        srv.send(
          buildDnsResponse(msg, 0, false, [
            { type: 1, ttl: 120, rdata: Buffer.from([10, 20, 30, 40]) },
            { type: 16, ttl: 60, rdata: Buffer.concat([Buffer.from([5]), Buffer.from("hello")]) },
            { type: 257, ttl: 60, rdata: Buffer.concat([Buffer.from([0, 5]), Buffer.from("iodef"), Buffer.from("mailto:a@b")]) },
          ]),
          rinfo.port,
          rinfo.address
        );
      }
    });

    await new Promise<void>((resolve) => srv.bind(0, "127.0.0.1", resolve));
    const port = (srv.address() as any).port;

    try {
      const rp = new DP.Resolver({ timeout: 1000, tries: 1 });
      const rc = new D.Resolver({ timeout: 1000, tries: 1 });
      rp.setServers([`127.0.0.1:${port}`]);
      rc.setServers([`127.0.0.1:${port}`]);

      expect(await rp.resolve4("example.test")).toEqual(["10.20.30.40"]);
      expect(await rp.resolve4("example.test", { ttl: true })).toEqual([
        { address: "10.20.30.40", ttl: 120 },
      ]);
      expect(await rp.resolve6("example.test")).toEqual(["2001:db8::1"]);
      expect(await rp.resolve6("example.test", { ttl: true })).toEqual([
        { address: "2001:db8::1", ttl: 90 },
      ]);
      expect(await rp.resolve("example.test")).toEqual(["10.20.30.40"]);
      expect(await rp.resolve("example.test", "MX")).toEqual([
        { priority: 10, exchange: "mail.mock.test" },
      ]);
      expect(await rp.resolveCname("example.test")).toEqual(["alias.example.test"]);
      expect(await rp.resolveNs("example.test")).toEqual(["ns1.mock.test"]);
      expect(await rp.resolvePtr("1.0.0.127.in-addr.arpa")).toEqual(["host.mock.test"]);
      expect(await rp.reverse("1.2.3.4")).toEqual(["host.mock.test"]);
      expect(lastQueries[lastQueries.length - 1]).toEqual({
        qname: "4.3.2.1.in-addr.arpa",
        qtype: 12,
      });
      expect(await rp.reverse("::1")).toEqual(["host.mock.test"]);
      expect(lastQueries[lastQueries.length - 1].qname).toBe(
        "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.ip6.arpa"
      );
      expect(await rp.resolveMx("example.test")).toEqual([
        { priority: 10, exchange: "mail.mock.test" },
      ]);
      expect(await rp.resolveTxt("example.test")).toEqual([["foo", "bar"]]);
      expect(await rp.resolveSrv("example.test")).toEqual([
        { name: "srv.mock.test", port: 8443, priority: 1, weight: 5 },
      ]);
      expect(await rp.resolveSoa("example.test")).toEqual({
        nsname: "ns1.mock.test",
        hostmaster: "admin.mock.test",
        serial: 20261125,
        refresh: 3600,
        retry: 600,
        expire: 86400,
        minttl: 300,
      });
      expect(await rp.resolveNaptr("example.test")).toEqual([
        {
          flags: "u",
          service: "E2U+sip",
          regexp: "!^.*$!sip:x@mock.test!",
          replacement: "next.mock.test",
          order: 100,
          preference: 10,
        },
      ]);
      expect(await rp.resolveCaa("example.test")).toEqual([
        { critical: 128, issue: "ca.example" },
      ]);

      const tlsa = await rp.resolveTlsa("example.test");
      expect(tlsa.length).toBe(1);
      expect(tlsa[0].certUsage).toBe(3);
      expect(tlsa[0].selector).toBe(1);
      expect(tlsa[0].match).toBe(1);
      expect(tlsa[0].data).toBeInstanceOf(ArrayBuffer);
      expect(Array.from(new Uint8Array(tlsa[0].data))).toEqual([0xaa, 0xbb, 0xcc, 0xdd]);

      expect(await rp.resolveAny("example.test")).toEqual([
        { type: "A", address: "10.20.30.40", ttl: 120 },
        { type: "TXT", entries: ["hello"] },
        { type: "CAA", critical: 0, iodef: "mailto:a@b" },
      ]);

      // Callback resolver + top-level default dns.* sanity
      const [cbA] = await cbCall<any>((cb) => rc.resolve4("example.test", cb));
      expect(cbA).toEqual(["10.20.30.40"]);
      const [cbMx] = await cbCall<any>((cb) => rc.resolve("example.test", "MX", cb));
      expect(cbMx).toEqual([{ priority: 10, exchange: "mail.mock.test" }]);
      const [cbRev] = await cbCall<any>((cb) => rc.reverse("1.2.3.4", cb));
      expect(cbRev).toEqual(["host.mock.test"]);

      // Validation errors
      expect(() => rc.resolve("example.test", "BADTYPE", () => {})).toThrow();
      let revErr: any;
      try {
        rc.reverse("not-ip", () => {});
      } catch (e) {
        revErr = e;
      }
      expect(revErr?.code).toBe("EINVAL");
    } finally {
      srv.close();
    }
  });

  it("retries truncated UDP responses over TCP and maps error codes cleanly", async () => {
    // 1. Bind TCP first, then UDP to the same port for TC=1 TCP fallback
    let udpSrv: dgram.Socket | null = null;
    let tcpSrv: net.Server | null = null;
    let port = 0;

    for (let attempt = 0; attempt < 5; attempt++) {
      tcpSrv = net.createServer((socket) => {
        let buf = Buffer.alloc(0);
        socket.on("data", (chunk: Buffer) => {
          buf = Buffer.concat([buf, chunk]);
          if (buf.length >= 2) {
            const msgLen = buf.readUInt16BE(0);
            if (buf.length >= 2 + msgLen) {
              const req = buf.subarray(2, 2 + msgLen);
              const resp = buildDnsResponse(req, 0, false, [
                { type: 1, ttl: 60, rdata: Buffer.from([192, 0, 2, 77]) },
              ]);
              const prefix = Buffer.alloc(2);
              prefix.writeUInt16BE(resp.length, 0);
              socket.end(Buffer.concat([prefix, resp]));
            }
          }
        });
      });
      await new Promise<void>((resolve) => tcpSrv!.listen(0, "127.0.0.1", resolve));
      port = (tcpSrv.address() as any).port;

      udpSrv = dgram.createSocket("udp4");
      try {
        await new Promise<void>((resolve, reject) => {
          udpSrv!.once("error", reject);
          udpSrv!.bind(port, "127.0.0.1", () => {
            udpSrv!.removeListener("error", reject);
            resolve();
          });
        });
        break;
      } catch {
        udpSrv.close();
        udpSrv = null;
        await new Promise<void>((resolve) => tcpSrv!.close(() => resolve()));
        tcpSrv = null;
      }
    }

    expect(udpSrv).not.toBeNull();
    expect(tcpSrv).not.toBeNull();

    let mode: "tc" | "nxdomain" | "nodata" | "servfail" | "refused" | "drop" = "tc";
    udpSrv!.on("message", (msg, rinfo) => {
      if (mode === "drop") return;
      if (mode === "tc") {
        udpSrv!.send(buildDnsResponse(msg, 0, true, []), rinfo.port, rinfo.address);
      } else if (mode === "nxdomain") {
        udpSrv!.send(buildDnsResponse(msg, 3, false, []), rinfo.port, rinfo.address);
      } else if (mode === "nodata") {
        udpSrv!.send(buildDnsResponse(msg, 0, false, []), rinfo.port, rinfo.address);
      } else if (mode === "servfail") {
        udpSrv!.send(buildDnsResponse(msg, 2, false, []), rinfo.port, rinfo.address);
      } else if (mode === "refused") {
        udpSrv!.send(buildDnsResponse(msg, 5, false, []), rinfo.port, rinfo.address);
      }
    });

    try {
      const rp = new DP.Resolver({ timeout: 250, tries: 1 });
      rp.setServers([`127.0.0.1:${port}`]);

      mode = "tc";
      expect(await rp.resolve4("tcp-fallback.test")).toEqual(["192.0.2.77"]);

      mode = "nxdomain";
      await expect(rp.resolve4("missing.test")).rejects.toMatchObject({
        code: "ENOTFOUND",
        syscall: "queryA",
        hostname: "missing.test",
      });

      mode = "nodata";
      await expect(rp.resolve4("empty.test")).rejects.toMatchObject({
        code: "ENODATA",
        syscall: "queryA",
      });

      mode = "servfail";
      await expect(rp.resolve4("servfail.test")).rejects.toMatchObject({
        code: "ESERVFAIL",
      });

      mode = "refused";
      await expect(rp.resolve4("refused.test")).rejects.toMatchObject({
        code: "EREFUSED",
      });

      mode = "drop";
      await expect(rp.resolve4("timeout.test")).rejects.toMatchObject({
        code: "ETIMEOUT",
      });

      // Cancellation while pending
      const rc = new DP.Resolver({ timeout: 5000, tries: 1 });
      rc.setServers([`127.0.0.1:${port}`]);
      const pending = rc.resolve4("cancelled.test");
      setTimeout(() => rc.cancel(), 25);
      await expect(pending).rejects.toMatchObject({
        code: "ECANCELLED",
      });
    } finally {
      udpSrv?.close();
      await new Promise<void>((resolve) => tcpSrv?.close(() => resolve()));
    }
  });
});
