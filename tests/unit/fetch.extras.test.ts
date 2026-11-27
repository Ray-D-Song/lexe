import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { gzipSync } from "node:zlib";
import { spawnCapture } from "./test-utils";

const ROOT_CA = `-----BEGIN CERTIFICATE-----
MIIEqjCCApICCQDXbtzmd5Xr1jANBgkqhkiG9w0BAQsFADAXMRUwEwYDVQQDDAxU
ZXN0IFJvb3QgQ0EwHhcNMjYwNzI5MDY1NTA4WhcNMzYwNzI2MDY1NTA4WjAXMRUw
EwYDVQQDDAxUZXN0IFJvb3QgQ0EwggIiMA0GCSqGSIb3DQEBAQUAA4ICDwAwggIK
AoICAQC3RvbgOVcbeWjKnl8jQbJyWqx0S+5G+0vhQvZESla30MaRTc6lP68u6qEH
VT4bt0gTM55NcRGe+tNSNAtcvvZLXBh0hOIIdRjyH0FZQgsRbIwtDSgAW2cU6H5M
eixDpraDjCH5V+iSKXbb6JJYsqdssGWdk1p6LWpQm7LH1JPTmmRDtmAX/JSSGEqr
8ZrcTliivC6Zegsk6EkwtwTdUk9QJtmFkCk1J2ub0D4NrI9eQteQGQpRSuMQlceL
f/pW0lqNK/aRkccGq4RsAHW1KrcLQhKmyg6fHBXrjNd0AxWtPOosy0eeBufJceSU
kt6DJB5gGmYUMcEV34ZpVGaj9EyKDjJKuHRIGy0iokGXCSVY+I9zNNXztYd3tgSb
3RMV4nWmVwWu/cAuPuAwJdMAmrFiQmfW41kK4P2lSCLqGET6nY10CoUyEXYSP6C/
KqYP5zf5/MfLTWx2lbmjIl6y0+qMt7//LtUug2v1Mj38Pfog7v2mmKVMyhqdMk3N
42RfBBeowJsv2nR3tCadE+dJ8zAKAC/9dLTp8XVI6h5v28caoNsOvpK6A87OicdZ
y2JZDWeFCtp+WTOvk3SoGLiG026XoXsuPiDrZ51ONRmka7gxKFCkxGV2uH2Inm6V
Irf/TIzPH0LvSotY7kfQXtmUWEI7N4taKD4sYBt0NGFsW0XpcwIDAQABMA0GCSqG
SIb3DQEBCwUAA4ICAQAIn5nKprNnmfRe6mhWoQHAzgRhtrIsIswmE1L03T56IvrW
5yySoPwLtkScAyjEiSo9WDnRHKgGRmusTnwICIXW0Gx9qJdDya2JAbqP/Gfd4lGq
qI3yvsdS2aVLzkITc1EKenx5sbdWLeION84jqliw05I2Q+UCDyOzjWze2DuPyuPl
hihlg7zHh/zYmpdyGj0y5XlvqiEeD0VZT7a6alp8TPryYPwQpf+jKVeuw05R+ZgA
nuFIRLsYUcSP64Z1J1lYiL1mRZq1aeenOI8zQ/6wLC3guv754CGLH0BZOzokIryM
xD/V+C/KKQoYHVKCag0HK95H2UKSxDE906rj0pm+mNNJCUDeoCz+v+TEChiaWOPB
X7m/U1P2X35So5scnGN1lSwTwGQ1QrsYozlpGzSM5i0h0gy+Qf3lei1sg8H3HEnK
+czQ2rKnQoZzCTKImx0/Yd0xFOvXoyzPBz9E/bg7jdgdnxLNTMx6GhnnRQAHYHqY
gAeF3f/IfltLQvVcB4WxYW57S7oBT+JzpoqsMX5MElUDLlpa3wT9TqN0JUAhJBmh
tjj9WQeLQp7h2XweHxuL8NpzlQto/XF8Lslq7BrFvkL2FHBiA/3v/17CP0oalvcD
oyiQzVibZUdsZx0xt5sGLquaJ70W99E8O9EkEPPBkIKA4gyuCF734l+BbNVcaA==
-----END CERTIFICATE-----`;

const SERVER_CERT = `-----BEGIN CERTIFICATE-----
MIIDzDCCAbSgAwIBAgIJAP81CsuTZAHqMA0GCSqGSIb3DQEBCwUAMBcxFTATBgNV
BAMMDFRlc3QgUm9vdCBDQTAeFw0yNjA3MjkwNjU1MDhaFw0zNjA3MjYwNjU1MDha
MBQxEjAQBgNVBAMMCWxvY2FsaG9zdDCCASIwDQYJKoZIhvcNAQEBBQADggEPADCC
AQoCggEBALnOIO61ZYwx226Dru8Joc1KvSpZO9I4aIQUcj6sjOR0A6Z5OtqE0JDz
T0ORQSJ3uFz2MNX2n2SlficBipQtrvVFQ13D6/Qeyif0EBoM4JfIRmC5zx793peI
ut83WFkYpbdAk3i0ogGN3aJnAHHhoggzcqXjR4Z1B9XT0qVdRK9GylpOw/8AUa0X
xmtN+Y58s6JjB4F1UmOIqCm9jos718+CActxXnLiOjQzjQXr33Z7SzpFi/kd3fQZ
Lx55+jE4WyEeX2cucr/9OVkEvtdAQafnBdnOuFPTjboXrLIRn+ByzaVekNMZZXb/
15/4qrZ2fJfuNryWXPjZPEY8enBoMxcCAwEAAaMeMBwwGgYDVR0RBBMwEYcEfwAA
AYIJbG9jYWxob3N0MA0GCSqGSIb3DQEBCwUAA4ICAQCH9rFR+5Q1cWDpS8F0wqsG
HYXClmzqvr5TMym00jNGYBUublVQ+lF2lsh9e/dvxVTVC4zi19xqK7dSglbNhi4L
0A20l2UNFIbiQ/lDbgpnuwWDigAdlZOzAW5tT3doF9YtEGgcsMhDo7paGqCp2zrg
eujgHXbDUOc729AfTQwW3EEjBfvZ+Vs0+UffVhRoxmQwCSAejKHpq0kfAvoJlULO
yYGLH/78HukO1Y9J89T7wic7ZKdSgTnuyjD7I8B+Yrsu6OgOT0TQG8oOMvez9H+j
k+Sk8Duuj7VF/+i1U+zbvindwiMOW7vkEUExWNncZRVXSlPmpcsHyoGhKs36PXNF
amC6loCgTm6MDt4t0xa1wod8A+k7mmWI69aMLMcU3nMyZQzt52mzdoUNxr41BONC
WPBrGIPCj8T/faSm6xO2028lRhNiAf+oZMSHI4r0Q7A98SneOFadRpCr+GljyQC5
iDaQ96Ith6N1gg1RCC+CIxxadjOxfYE3AmmIulXsHT2+p7SG+aBuVXdxD5AY02By
dEg7FnyiYN1ocP/9VGfcpBsHOw5+GGL7wbNJspXZ+43g42k+dmHcHJbyw3+agdNV
PQ6HXlblnFBkLE8TqEaM4t45HdFq7D5GFPhjg+lRokndNHBQtC5Avb9NFTbkdcD/
BTwa1iK34p26FjUyi59NPw==
-----END CERTIFICATE-----`;

const SERVER_KEY = `-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEAuc4g7rVljDHbboOu7wmhzUq9Klk70jhohBRyPqyM5HQDpnk6
2oTQkPNPQ5FBIne4XPYw1fafZKV+JwGKlC2u9UVDXcPr9B7KJ/QQGgzgl8hGYLnP
Hv3el4i63zdYWRilt0CTeLSiAY3domcAceGiCDNypeNHhnUH1dPSpV1Er0bKWk7D
/wBRrRfGa035jnyzomMHgXVSY4ioKb2OizvXz4IBy3FecuI6NDONBevfdntLOkWL
+R3d9BkvHnn6MThbIR5fZy5yv/05WQS+10BBp+cF2c64U9ONuhesshGf4HLNpV6Q
0xlldv/Xn/iqtnZ8l+42vJZc+Nk8Rjx6cGgzFwIDAQABAoIBAAHKsxaaOTh44pzW
OEGl3iGUru5RnfoRATEE4XUpRNKPK9nXIXIqBbjgH8uPAwnKDcUXurDViAP4dJb7
tNJo/l7vJA8rbIMWKOzptZ4Ksb/E9qjIIe8yk4ryjprIddmmGzMBNGrdkxJgw9pl
E+QFQQLDPD/NSUtKpaCyQj3GAHuQNviZFx1oOApv6QXX4ZLNjz2risFuDeW8j0As
xUwXvBGpZtmNn/jb6ZD/fgW+QD6inso6i8nM4qoQanpmfI0jRFcs0bY6iLorbe46
6bOmD9bhoW1IU0ReOiGX1wSoR010LSzy9WI3WNFsmB3K1ApsvyoqJ1CceE9KpCFp
F17lskECgYEA8iO3H8yqOxMeNY+IOWlCzmA5LEyrIbpBQRhLVRgFH0QN8tZ7JGWf
mM3xSY94mm6hciB/NWr2Siq5L2ybgg8ZeB2JaVWAZQiZ6RMFe4vSQYiJjdqXsyME
aDz0woy9UsF51r1PupT6RtUf39JYeiOrnB0YOqyxb04oP36DVwOyUx0CgYEAxHDl
h6QGHAGw34pkxYoIdcXlohqJpwZCJFcbwTObHG9QNKVgi622TmXUe6vO0aI8Gy/q
yXP1KUaBzfJJ1KPQhAzVaX4jIn6dmiICh+2M/h7FCkKOSpSo3kU/1OGQRFFpUS9L
diwAud1AjyPtOCrG/hXYoDU/Sh4nf//v6RmlNMMCgYBB4kXpeR64hDMsD6nZNTLg
RH5J93J48tzegWW9+ZMrNrE06fcHqD8MrLyCb285VfDHxN/YAQqSk51fsgV+Fe2w
KRlCd+k/lF+Mc4iCXUEQY75JwriUHWsNkchnRzQcWFh2fXw8IF0Y+NR7ahh440vG
ZoQ5MtP0nSeGXz4NZwwWYQKBgQCrPF5YprpMn4y2z8SYDYLiZueNLuTfG618UBJ5
j6qeVsnrml1WZM6N+GYIh/N4+Zq7eEGJ6fLDUBrGk3WYTlPtR+KgWrv1zblEKnFa
W9CTk+7eyrWOfZfWaTcDMry3UOAlTLzS/fiEDEHChOa8F03vHsv+m2vYCAEWqttO
rnQ8UQKBgQDwHH5UjSleCmItLEN8Hu691Mz2eLmSaBsBSP6cIcGXK9PPKTLsYOxW
hd6bqISuvzgF2o6gG8mTzjmmeTQhyrk7grzFT0lC7yAXDp5LopGUeUowOVdeASnU
p4X7MxD+uxEt52/IxZrn0dtasypzePJM66hBc+jEcZjMblwQJxohXw==
-----END RSA PRIVATE KEY-----`;

let tmpDir: string;
let httpServer: http.Server;
let baseUrl: string;
let reqCounter = 0;

function sha256Sri(data: string | Uint8Array): string {
  const b64 = crypto.createHash("sha256").update(data).digest("base64");
  return `sha256-${b64}`;
}

function sha512Sri(data: string | Uint8Array): string {
  const b64 = crypto.createHash("sha512").update(data).digest("base64");
  return `sha512-${b64}`;
}

beforeAll((done) => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lexe-fetch-extras-"));
  httpServer = http.createServer((req, res) => {
    const reqUrl = new URL(req.url ?? "/", "http://127.0.0.1");
    if (reqUrl.pathname === "/trickle") {
      res.writeHead(200, { "Content-Type": "text/plain" });
      let i = 0;
      const timer = setInterval(() => {
        res.write(`c${i}-`);
        i += 1;
        if (i === 4) {
          clearInterval(timer);
          res.end("end");
        }
      }, 2);
      return;
    }
    if (reqUrl.pathname === "/gzip") {
      const payload = gzipSync(Buffer.from("compressed-integrity-body"));
      res.writeHead(200, {
        "Content-Type": "text/plain",
        "Content-Encoding": "gzip",
        "Content-Length": String(payload.length),
      });
      res.end(payload);
      return;
    }
    if (reqUrl.pathname === "/vary") {
      const custom = req.headers["x-lang"] ?? "en";
      res.writeHead(200, {
        "Content-Type": "text/plain",
        Vary: "X-Lang",
      });
      res.end(`lang:${custom}`);
      return;
    }
    if (reqUrl.pathname === "/vary-star") {
      res.writeHead(200, { "Content-Type": "text/plain", Vary: "*" });
      res.end("no-cache");
      return;
    }
    if (reqUrl.pathname === "/partial") {
      res.writeHead(206, { "Content-Type": "text/plain" });
      res.end("partial");
      return;
    }
    if (reqUrl.pathname === "/not-found") {
      res.writeHead(404, { "Content-Type": "text/plain" });
      res.end("missing");
      return;
    }
    if (reqUrl.pathname === "/counter") {
      reqCounter += 1;
      res.writeHead(200, { "Content-Type": "text/plain" });
      res.end(`hit-${reqCounter}`);
      return;
    }
    res.writeHead(200, { "Content-Type": "text/plain" });
    res.end("ok-body");
  });

  httpServer.listen(0, "127.0.0.1", () => {
    const addr = httpServer.address() as { port: number };
    baseUrl = `http://127.0.0.1:${addr.port}`;
    done();
  });
});

afterAll(() => {
  httpServer?.close();
  fs.rmSync(tmpDir, { recursive: true, force: true });
});

describe("Response and Request clone() + duplex edge cases", () => {
  it("clones Response after accessing .body without stalling and throws when locked", async () => {
    const res = await fetch(`${baseUrl}/trickle`);
    const bodyRef = res.body;
    expect(bodyRef).not.toBeNull();
    const cloned = res.clone();
    const [a, b] = await Promise.all([res.text(), cloned.text()]);
    expect(a).toBe("c0-c1-c2-c3-end");
    expect(b).toBe("c0-c1-c2-c3-end");

    const resLocked = new Response("hello");
    const reader = resLocked.body!.getReader();
    expect(() => resLocked.clone()).toThrow();
    reader.releaseLock();

    const fetchedLocked = await fetch(`${baseUrl}/trickle`);
    const r2 = fetchedLocked.body!.getReader();
    expect(() => fetchedLocked.clone()).toThrow();
    r2.releaseLock();
    await fetchedLocked.body!.cancel();
  });

  it("handles concurrent branch cancellation and stress cloning", async () => {
    const rs = new ReadableStream({
      pull(ctrl) {
        ctrl.enqueue(new TextEncoder().encode("abc"));
        ctrl.close();
      },
    });
    const r1 = new Response(rs);
    const r2 = r1.clone();
    const [, text] = await Promise.all([r1.body!.cancel("done"), r2.text()]);
    expect(text).toBe("abc");

    for (let i = 0; i < 20; i += 1) {
      const f = await fetch(`${baseUrl}/trickle`);
      const c1 = f.clone();
      const c2 = c1.clone();
      const [t0, t1, t2] = await Promise.all([f.text(), c1.text(), c2.text()]);
      expect(t0).toBe("c0-c1-c2-c3-end");
      expect(t1).toBe(t0);
      expect(t2).toBe(t0);
    }
  });

  it("validates duplex for ReadableStream Request/fetch and prevents double fetch(Request)", async () => {
    const rs = new ReadableStream({
      start(c) {
        c.enqueue(new Uint8Array([1, 2, 3]));
        c.close();
      },
    });
    expect(() => {
      new Request(baseUrl, { method: "POST", body: rs });
    }).toThrow(/duplex/);
    expect(() => {
      new Request(baseUrl, {
        method: "POST",
        body: "ok",
        duplex: "invalid",
      } as RequestInit);
    }).toThrow();

    const validReq = new Request(baseUrl, { method: "POST", body: "ping" });
    expect(validReq.duplex).toBe("half");
    const s = validReq.body;
    expect(s).not.toBeNull();
    const clonedReq = validReq.clone();
    expect(await Promise.all([validReq.text(), clonedReq.text()])).toEqual([
      "ping",
      "ping",
    ]);

    const fetchedReq = new Request(baseUrl, { method: "POST", body: "once" });
    await fetch(fetchedReq);
    expect(fetchedReq.bodyUsed).toBe(true);
    await expect(fetch(fetchedReq)).rejects.toThrow();
  });
});

describe("HTTP/2 streaming request upload", () => {
  it("streams chunks, handles empty stream, aborts, and resets on JS error over ALPN h2", async () => {
    const caFile = path.join(tmpDir, "root-ca.pem");
    const certFile = path.join(tmpDir, "server-cert.pem");
    const keyFile = path.join(tmpDir, "server-key.pem");
    fs.writeFileSync(caFile, ROOT_CA);
    fs.writeFileSync(certFile, SERVER_CERT);
    fs.writeFileSync(keyFile, SERVER_KEY);

    const serverScript = `
      const http2 = require("node:http2");
      const fs = require("node:fs");
      const server = http2.createSecureServer({
        key: fs.readFileSync(${JSON.stringify(keyFile)}),
        cert: fs.readFileSync(${JSON.stringify(certFile)}),
        allowHTTP1: false,
      });
      let sawReset = false;
      server.on("stream", (stream, headers) => {
        stream.on("error", () => {});
        const p = headers[":path"];
        if (p === "/echo-h2") {
          const chunks = [];
          let aborted = false;
          stream.on("aborted", () => { aborted = true; });
          stream.on("data", (c) => chunks.push(c));
          stream.on("end", () => {
            if (aborted || stream.destroyed || stream.closed) return;
            stream.respond({ ":status": 200, "content-type": "text/plain" });
            stream.end("h2:" + Buffer.concat(chunks).toString("utf8"));
          });
          return;
        }
        if (p === "/early-error") {
          if (stream.destroyed || stream.closed) return;
          stream.respond({ ":status": 200, "content-type": "text/plain" });
          const mark = () => { if (!stream.readableEnded) sawReset = true; };
          stream.on("error", mark);
          stream.on("aborted", mark);
          stream.on("close", mark);
          return;
        }
        if (p === "/check-reset") {
          if (stream.destroyed || stream.closed) return;
          stream.respond({ ":status": 200, "content-type": "text/plain" });
          stream.end(sawReset ? "reset-observed" : "no-reset");
        }
      });
      server.listen(0, "127.0.0.1", () => {
        console.log("H2_PORT:" + server.address().port);
      });
    `;

    const h2Proc = spawn("node", ["-e", serverScript]);
    const port = await new Promise<number>((resolve, reject) => {
      let buf = "";
      h2Proc.stdout.on("data", (d) => {
        buf += d.toString();
        const m = buf.match(/H2_PORT:(\d+)/);
        if (m) resolve(Number(m[1]));
      });
      h2Proc.stderr.on("data", (d) => reject(new Error(d.toString())));
      h2Proc.on("exit", (code) => reject(new Error(`node h2 server exited ${code}`)));
    });

    try {
      // Child process loads LEXE_EXTRA_CA_CERTS so FETCH_HTTP_CLIENT negotiates
      // TLS ALPN h2 by default without requiring LEXE_HTTP_VERSION=2.
      const script = `
        const port = ${port};
        const chunks = ["alpha-", "beta-", "gamma"];
        let idx = 0;
        const stream = new ReadableStream({
          pull(c) {
            return new Promise((r) => {
              setTimeout(() => {
                if (idx < chunks.length) c.enqueue(new TextEncoder().encode(chunks[idx++]));
                else c.close();
                r();
              }, 2);
            });
          }
        });
        const res = await fetch("https://127.0.0.1:" + port + "/echo-h2", {
          method: "POST",
          body: stream,
          duplex: "half",
        });
        const txt = await res.text();
        if (txt !== "h2:alpha-beta-gamma") throw new Error("unexpected h2 echo: " + txt);

        const entries = performance.getEntriesByType("resource");
        const proto = entries[entries.length - 1]?.nextHopProtocol;
        if (proto !== "h2") throw new Error("expected h2 protocol, got " + proto);

        const empty = new ReadableStream({ start(c) { c.close(); } });
        const emptyRes = await fetch("https://127.0.0.1:" + port + "/echo-h2", {
          method: "POST",
          body: empty,
          duplex: "half",
        });
        if ((await emptyRes.text()) !== "h2:") throw new Error("empty h2 upload failed");

        const ac = new AbortController();
        const stalled = new ReadableStream({ start() {} });
        const pAbort = fetch("https://127.0.0.1:" + port + "/echo-h2", {
          method: "POST",
          body: stalled,
          duplex: "half",
          signal: ac.signal,
        });
        setTimeout(() => ac.abort("abort-h2"), 5);
        let abortedOk = false;
        try { await pAbort; } catch (e) { abortedOk = e === "abort-h2"; }
        if (!abortedOk) throw new Error("h2 abort failed");

        const failing = new ReadableStream({
          start(c) {
            c.enqueue(new TextEncoder().encode("part-1"));
            setTimeout(() => c.error(new Error("boom-upload")), 10);
          }
        });
        const earlyRes = await fetch("https://127.0.0.1:" + port + "/early-error", {
          method: "POST",
          body: failing,
          duplex: "half",
        });
        await new Promise((r) => setTimeout(r, 40));
        const resetCheck = await fetch("https://127.0.0.1:" + port + "/check-reset");
        console.log("H2_OK", earlyRes.status, await resetCheck.text());
      `;

      const { code, stdout, stderr } = await spawnCapture(
        process.argv0,
        ["-e", script],
        {
          env: {
            ...process.env,
            LEXE_EXTRA_CA_CERTS: caFile,
          },
        }
      );
      expect(stderr).toBe("");
      expect(code).toBe(0);
      expect(stdout).toContain("H2_OK 200 reset-observed");
    } finally {
      h2Proc.kill();
    }
  });
});

describe("SRI integrity verification", () => {
  it("validates sha256/sha512 across http, gzip, data, and blob schemes", async () => {
    const req = new Request(baseUrl, { integrity: sha256Sri("ok-body") });
    expect(req.integrity).toBe(sha256Sri("ok-body"));

    const res = await fetch(baseUrl, { integrity: sha256Sri("ok-body") });
    expect(await res.text()).toBe("ok-body");

    await expect(
      fetch(baseUrl, {
        integrity: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
      })
    ).rejects.toThrow(/Integrity/);

    const gzRes = await fetch(`${baseUrl}/gzip`, {
      integrity: sha512Sri("compressed-integrity-body"),
    });
    expect(await gzRes.text()).toBe("compressed-integrity-body");

    // Strongest tier wins per SRI spec.
    await expect(
      fetch(baseUrl, {
        integrity: `${sha256Sri("ok-body")} sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==`,
      })
    ).rejects.toThrow(/Integrity/);

    const dataRes = await fetch("data:text/plain,hello", {
      integrity: `sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= ${sha512Sri("hello")}?v=1`,
    });
    expect(await dataRes.text()).toBe("hello");

    const blobUrl = URL.createObjectURL(new Blob(["blob-sri"]));
    try {
      const blobRes = await fetch(blobUrl, { integrity: sha256Sri("blob-sri") });
      expect(await blobRes.text()).toBe("blob-sri");
    } finally {
      URL.revokeObjectURL(blobUrl);
    }
  });
});

describe("CacheStorage and Cache (caches)", () => {
  it("supports CacheStorage open/has/keys/delete/match and non-constructible errors", async () => {
    expect(caches instanceof CacheStorage).toBe(true);
    expect(() => new (Cache as unknown as { new (): unknown })()).toThrow();
    expect(() => new (CacheStorage as unknown as { new (): unknown })()).toThrow();

    const cacheName = `test-cache-${Date.now()}`;
    const cache = await caches.open(cacheName);
    expect(cache instanceof Cache).toBe(true);
    expect(await caches.has(cacheName)).toBe(true);
    expect(await caches.keys()).toContain(cacheName);

    const url = `${baseUrl}/items?q=1#frag`;
    await cache.put(url, new Response("cached-v1", { status: 201 }));
    const hit1 = await cache.match(url);
    const hit2 = await caches.match(`${baseUrl}/items?q=1#other`);
    expect(hit1?.status).toBe(201);
    expect(await hit1!.text()).toBe("cached-v1");
    expect(await hit2!.text()).toBe("cached-v1");

    const hitIgnoreSearch = await cache.match(`${baseUrl}/items`, {
      ignoreSearch: true,
    });
    expect(await hitIgnoreSearch!.text()).toBe("cached-v1");

    expect(await cache.match(`${baseUrl}/items?q=1`, { ignoreMethod: false })).toBeDefined();
    const postMiss = await cache.match(
      new Request(url, { method: "POST", body: "x" })
    );
    expect(postMiss).toBeUndefined();
    const postHit = await cache.match(
      new Request(url, { method: "POST", body: "x" }),
      { ignoreMethod: true }
    );
    expect(await postHit!.text()).toBe("cached-v1");

    const keys = await cache.keys();
    expect(keys.length).toBe(1);
    expect(keys[0].url).toBe(url);

    await cache.add(`${baseUrl}/counter`);
    const all = await cache.matchAll();
    expect(all.length).toBe(2);

    expect(await cache.delete(url)).toBe(true);
    expect(await cache.match(url)).toBeUndefined();
    expect(await caches.delete(cacheName)).toBe(true);
    expect(await caches.has(cacheName)).toBe(false);
  });

  it("enforces Vary matching and rejects unsupported put/add inputs", async () => {
    const cache = await caches.open(`vary-cache-${Date.now()}`);
    await cache.addAll([`${baseUrl}/vary`]);
    const matchedEn = await cache.match(
      new Request(`${baseUrl}/vary`, { headers: { "X-Lang": "en" } })
    );
    const missedFr = await cache.match(
      new Request(`${baseUrl}/vary`, { headers: { "X-Lang": "fr" } })
    );
    expect(matchedEn).toBeUndefined(); // stored req had no X-Lang header
    const matchedDefault = await cache.match(`${baseUrl}/vary`);
    expect(await matchedDefault!.text()).toBe("lang:en");
    expect(missedFr).toBeUndefined();

    await expect(
      cache.put(
        new Request(baseUrl, { method: "POST", body: "x" }),
        new Response("no")
      )
    ).rejects.toThrow();
    await expect(
      cache.put("file:///tmp/x.txt", new Response("no"))
    ).rejects.toThrow();
    await expect(cache.add(`${baseUrl}/not-found`)).rejects.toThrow();
    await expect(cache.add(`${baseUrl}/partial`)).rejects.toThrow();
    await expect(cache.add(`${baseUrl}/vary-star`)).rejects.toThrow();
    await caches.delete(cacheNameFromAll(await caches.keys()));
  });
});

function cacheNameFromAll(names: string[]): string {
  return names[names.length - 1] ?? "";
}

describe("fetch file:// protocol", () => {
  it("reads local files with GET/HEAD and maps missing/dir/invalid cases", async () => {
    const jsonPath = path.join(tmpDir, "hello%20world.json");
    fs.writeFileSync(jsonPath, JSON.stringify({ hello: "file" }));
    const fileUrl = `${pathToFileURL(jsonPath).href}?v=1#hash`;

    const res = await fetch(fileUrl);
    expect(res.status).toBe(200);
    expect(res.ok).toBe(true);
    expect(res.headers.get("content-type")).toContain("application/json");
    expect(res.headers.get("content-length")).toBe(
      String(Buffer.byteLength('{"hello":"file"}'))
    );
    expect(await res.json()).toEqual({ hello: "file" });

    const headRes = await fetch(fileUrl, { method: "HEAD" });
    expect(headRes.status).toBe(200);
    expect(headRes.body).toBeNull();
    expect(headRes.headers.get("content-length")).toBe(
      String(Buffer.byteLength('{"hello":"file"}'))
    );

    const localhostUrl = fileUrl.replace(/^file:\/\//, "file://localhost");
    const localhostRes = await fetch(localhostUrl);
    expect(await localhostRes.json()).toEqual({ hello: "file" });

    const missingRes = await fetch(pathToFileURL(path.join(tmpDir, "nope.txt")).href);
    expect(missingRes.status).toBe(404);
    expect(missingRes.ok).toBe(false);

    const dirRes = await fetch(pathToFileURL(tmpDir).href);
    expect(dirRes.status).toBe(403);
    expect(dirRes.ok).toBe(false);

    await expect(fetch("file://remote.example/etc/passwd")).rejects.toThrow();
    await expect(
      fetch(fileUrl, { method: "POST", body: "x" })
    ).rejects.toThrow();

    const sriFile = await fetch(fileUrl, {
      integrity: sha256Sri('{"hello":"file"}'),
    });
    expect(await sriFile.json()).toEqual({ hello: "file" });
  });
});
