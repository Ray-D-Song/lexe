import defaultImport from "node:tls";
import legacyImport from "tls";

it("node:tls should be the same as tls", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

it("should export connect and createServer", () => {
  expect(typeof legacyImport.connect).toBe("function");
  expect(typeof legacyImport.createServer).toBe("function");
});

it("should export TLSSocket and Server classes", () => {
  expect(typeof legacyImport.TLSSocket).toBe("function");
  expect(typeof legacyImport.Server).toBe("function");
});

it("should export constants", () => {
  expect(legacyImport.CLIENT_RENEG_LIMIT).toBe(3);
  expect(legacyImport.CLIENT_RENEG_WINDOW).toBe(600);
  expect(typeof legacyImport.DEFAULT_CIPHERS).toBe("string");
  expect(legacyImport.DEFAULT_ECDH_CURVE).toBe("auto");
  expect(legacyImport.DEFAULT_MIN_VERSION).toBe("TLSv1.2");
  expect(legacyImport.DEFAULT_MAX_VERSION).toBe("TLSv1.3");
});

it("ESM import works", async () => {
  const tls = await import("node:tls");
  expect(typeof tls.connect).toBe("function");
});

it("TLSSocket exposes setKeepAlive, setNoDelay, and live TLS getters", () => {
  const sock = new legacyImport.TLSSocket();
  expect(sock.encrypted).toBe(true);
  expect(sock.alpnProtocol).toBe(false);
  expect(sock.authorized).toBe(false);
  expect(Array.isArray(sock.autoSelectFamilyAttemptedAddresses)).toBe(true);
  expect(sock.setNoDelay(true)).toBe(sock);
  expect(sock.setKeepAlive(true, 1000)).toBe(sock);
  sock.destroy();
});

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

const ENCRYPTED_SERVER_KEY = `-----BEGIN ENCRYPTED PRIVATE KEY-----
MIIFLTBXBgkqhkiG9w0BBQ0wSjApBgkqhkiG9w0BBQwwHAQI/NRvK3c55EACAggA
MAwGCCqGSIb3DQIJBQAwHQYJYIZIAWUDBAEqBBBC3Lq/2yElkPXL8BDIsHChBIIE
0Od94Iy2+A/lvYbozNo7yS2TetCQx8aSqZUmURpqTHnkXld+odozoi73C4HB6NJp
ru0+kSDULX/w98dICU0nieBzObsXz/NsD2cGiUnDmnRuDBCoMhb9nqVXtGGXGoSB
x1aRoTfW1N23Euv86ECSyJq1o9CeM47kJVU8S8XOEcn0Wj7gC2SeHc+5ieOg7Aub
ZbEymWmcCp8NA/HDcTELOCKbhlkKOdeyMfeBrFNSfDXOKkS5zqQDsf+/wwtFFvAG
SD/nu0/JMIajKx5bdSvaGj00TU9Wo4UAUn/cAVxEAgyX5NNa2yNU4oW67lIEGxcZ
j78dLL06IDYI897lW61fJoays3MSY1pAeo+8heUBx2iz9Gs/BectvFW77BQhqjNZ
l2qlppJYIn2RPWYPzxQFqmpDe7kt2NxtkYPIr31WGYy0v1DIP4wVEgx27tN0k8ae
7fRCOh3xOTcAeRixZmqd/xfq9JeCrMNHJwsHj27OJGI8sbLZm9lNANtKF/+tyXRR
HyoC543YIRvsN440s+qx280PGkHiUHTu1XJDfPYjx35S5m54gFHiQTfqrXK+wasr
rX7GeUNZvjdInV+Cc4GLFpBnvDtd8te639Mri3uYzsvQMs1XbFECCzm/iBhxFjVp
lw9d8A4qa6/XqXHREcKxE8keiD7jYm5ZcHsjXWYDcWZRZBF/MNXGW9Bbu9C1XDMt
UbRTqkhEMrPDNzSID+zlyvo5oLyOIqzDuMKF4z2wyhml/l4SHWN3abzjGOU5njjN
k8S+QVoUbFvvPePM7awDbeyZHaJOkhb91ylEOYq+wL1VG4uU8vk1439RSjTu/6jd
NXIOLasc7VKjHVQug/D37lDyldIusw2IZoztclh62NEsNWZm2OCJ9YFhOeaePs6C
Fv/OCY+y17AlTprtHUIJum0xNqwK3Ev31hFHs8Bguhh5jgU9H1qZ0W+RCPQ+jGAm
jJCdnkKX++fCWeQl0gnzY11OJV7uxF0jyB+h2wWZdjAezL90mZbzpQRLgi7PK8K5
TshZosaUjDHGkmBI7oKE1IJWcOeUeOHqX4S7ZpEBRHFem1PhpVeNK2G4q1kUDTpM
V9AwExImCHrjjpmLCMYRvubguMx7Tl1Y65ZpHRRQ4/MMFtB6UNEt3Kb0DOENevLZ
xAbXzlnD4CXyzNyfezKLFn8vAGxN7DQ1fCa/opgI5/l56Kj4d/R4bxBC4pfv4gUs
vJNlKUCtEFPnwX6EYArkKxAlTSaow0lOqrm6hyYq3sItWSx/SYDm46US+wIldCMR
AxUw/gtRGL+t9VdJsilci7+NaG5rucCj4eFo/dAS46nGNG6DxvW/2zTjpiQCQAsX
ebdcSm8+2NORCbSH/nfD2znP3JjGmeJ66WER1Z/q409II1I3npoidd+eSLyffgQl
skgBijBeOPTPN4aLdUHWutGWo/hzgurT9EqpnxJcPFQisgQOwaZ7Ixi0f0YYUOtn
iMUYu2a7NjNEaJfQt2iEQ/WpvmKTrB/BLH5Ac+ADdTxVsv7zY7QRxk4NWGi1V+0p
dhKq8qd9+zHW6Vs7QRzLTP0gijh7wWH9gt0TvBhCkN7k+TC0IYUyQU9s3ubgL7U7
Uo07cnIDSFipBlBjVeq5OB3h20VNbJ1MP89cRn4USZlj
-----END ENCRYPTED PRIVATE KEY-----`;

const SERVER_PFX_B64 =
  "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";

it("exposes rootCertificates and getCACertificates", () => {
  expect(Array.isArray(legacyImport.rootCertificates)).toBe(true);
  expect(Object.isFrozen(legacyImport.rootCertificates)).toBe(true);
  expect(Array.isArray(legacyImport.getCACertificates("default"))).toBe(true);
  expect(() => (legacyImport as any).getCACertificates("bogus")).toThrow();
});

it("supports pfx and encrypted private key with passphrase", async () => {
  const s1 = legacyImport.createServer(
    { pfx: Buffer.from(SERVER_PFX_B64, "base64"), passphrase: "pfx-secret" },
    (sock) => sock.end("pfx-ok")
  );
  await new Promise<void>((resolve) => s1.listen(0, "127.0.0.1", resolve));
  try {
    const addr1 = s1.address() as any;
    const r1 = await new Promise<string>((resolve, reject) => {
      const c = legacyImport.connect({ host: "127.0.0.1", port: addr1.port, ca: ROOT_CA }, () => {
        let body = "";
        c.on("data", (d) => (body += d.toString()));
        c.on("end", () => resolve(body));
      });
      c.on("error", reject);
    });
    expect(r1).toBe("pfx-ok");
  } finally {
    await new Promise<void>((resolve) => s1.close(() => resolve()));
  }

  const s2 = legacyImport.createServer(
    { key: ENCRYPTED_SERVER_KEY, cert: SERVER_CERT, passphrase: "test-passphrase" },
    (sock) => sock.end("enc-ok")
  );
  await new Promise<void>((resolve) => s2.listen(0, "127.0.0.1", resolve));
  try {
    const addr2 = s2.address() as any;
    const r2 = await new Promise<string>((resolve, reject) => {
      const c = legacyImport.connect({ host: "127.0.0.1", port: addr2.port, ca: ROOT_CA }, () => {
        let body = "";
        c.on("data", (d) => (body += d.toString()));
        c.on("end", () => resolve(body));
      });
      c.on("error", reject);
    });
    expect(r2).toBe("enc-ok");
  } finally {
    await new Promise<void>((resolve) => s2.close(() => resolve()));
  }
});

it("supports async SNICallback, ALPNCallback, soft mTLS, write backpressure, and session resumption", async () => {
  const ctx = legacyImport.createSecureContext({ key: SERVER_KEY, cert: SERVER_CERT });
  let seenAlpn = "";
  const server = legacyImport.createServer(
    {
      key: SERVER_KEY,
      cert: SERVER_CERT,
      ca: ROOT_CA,
      requestCert: true,
      rejectUnauthorized: false,
      SNICallback(_servername: string, cb: (err: Error | null, c?: any) => void) {
        setTimeout(() => cb(null, ctx), 10);
      },
      ALPNCallback(info: { servername: string; protocols: string[] }) {
        seenAlpn = info.protocols.find((p) => p === "custom-proto") || "";
        return seenAlpn || undefined;
      },
    },
    (sock) => {
      expect(sock.authorized).toBe(false);
      expect(typeof sock.authorizationError).toBeTruthy();
      sock.on("data", () => {});
      sock.end("hi:" + (sock.isSessionReused() ? "reused" : "full"));
    }
  );
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const port = (server.address() as any).port;

    // First connection: ALPN + session capture + large write backpressure
    const { body: b1, session } = await new Promise<{ body: string; session: Buffer }>(
      (resolve, reject) => {
        let captured: Buffer | undefined;
        const c = legacyImport.connect(
          {
            host: "127.0.0.1",
            port,
            servername: "localhost",
            ca: ROOT_CA,
            ALPNProtocols: ["custom-proto", "http/1.1"],
          },
          () => {
            expect(c.alpnProtocol).toBe("custom-proto");
            expect(c.isSessionReused()).toBe(false);
            const big = Buffer.alloc(64 * 1024, 0x61);
            const fl = c.write(big);
            expect(fl).toBe(false);
            let drained = false;
            c.once("drain", () => {
              drained = true;
            });
            let body = "";
            c.on("data", (d) => (body += d.toString()));
            c.on("end", () => {
              expect(drained).toBe(true);
              resolve({ body, session: (captured || c.getSession() || Buffer.alloc(0)) as Buffer });
            });
          }
        );
        c.on("session", (s: Buffer) => {
          captured = s;
        });
        c.on("error", reject);
      }
    );
    expect(seenAlpn).toBe("custom-proto");
    expect(b1).toBe("hi:full");
    expect(session.length).toBeGreaterThan(0);

    // Second connection: resume using captured session
    const { body: b2, reused } = await new Promise<{ body: string; reused: boolean }>(
      (resolve, reject) => {
        const c = legacyImport.connect(
          {
            host: "127.0.0.1",
            port,
            servername: "localhost",
            ca: ROOT_CA,
            session,
          },
          () => {
            let body = "";
            c.on("data", (d) => (body += d.toString()));
            c.on("end", () => resolve({ body, reused: c.isSessionReused() }));
          }
        );
        c.on("error", reject);
      }
    );
    expect(reused).toBe(true);
    expect(b2).toBe("hi:reused");
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
