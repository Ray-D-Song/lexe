import assert from "node:assert/strict";
import fs from "node:fs";
import http2, {
  Http2ServerRequest,
  Http2ServerResponse,
  connect,
  constants,
  createSecureServer,
  createServer,
  getDefaultSettings,
  getPackedSettings,
  getUnpackedSettings,
  performServerHandshake,
  sensitiveHeaders,
} from "node:http2";
import path from "node:path";

const tlsDir = path.resolve(process.cwd(), "../../libs/lexe_test_tls/data");
const key = fs.readFileSync(path.join(tlsDir, "server.key"));
const cert = fs.readFileSync(path.join(tlsDir, "server.pem"));
const ca = fs.readFileSync(path.join(tlsDir, "root.pem"));

assert.equal(typeof Http2ServerRequest, "function");
assert.equal(typeof Http2ServerResponse, "function");
assert.equal(typeof performServerHandshake, "function");
assert.equal(typeof sensitiveHeaders, "symbol");
assert.equal(http2.Http2ServerRequest, Http2ServerRequest);
assert.equal(http2.Http2ServerResponse, Http2ServerResponse);

const defaults = getDefaultSettings();
assert.equal(defaults.headerTableSize, 4096);
const packed = getPackedSettings({ enablePush: false, maxConcurrentStreams: 32 });
const unpacked = getUnpackedSettings(packed);
assert.equal(unpacked.enablePush, false);
assert.equal(unpacked.maxConcurrentStreams, 32);

async function roundTrip(server, urlPrefix, clientOptions) {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  try {
    const res = await new Promise((resolve, reject) => {
      const client = connect(`${urlPrefix}://127.0.0.1:${port}`, clientOptions);
      client.on("error", reject);
      const req = client.request({
        [constants.HTTP2_HEADER_METHOD]: constants.HTTP2_METHOD_GET,
        [constants.HTTP2_HEADER_PATH]: "/compat-check",
      });
      let status;
      let customHeader;
      const chunks = [];
      req.on("response", (headers) => {
        status = headers[constants.HTTP2_HEADER_STATUS];
        customHeader = headers["x-compat-h2"];
      });
      req.on("data", (chunk) => chunks.push(chunk));
      req.on("end", () => {
        client.close(() => {
          resolve({
            status,
            customHeader,
            body: Buffer.concat(chunks).toString("utf8"),
          });
        });
      });
      req.on("error", reject);
      req.end();
    });
    return res;
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

function makeHandler() {
  return (req, res) => {
    assert.ok(req instanceof Http2ServerRequest);
    assert.ok(res instanceof Http2ServerResponse);
    assert.equal(req.httpVersion, "2.0");
    assert.equal(req.method, "GET");
    assert.equal(req.url, "/compat-check");
    res.setHeader("x-compat-h2", "ok");
    res.writeHead(201, { "content-type": "text/plain" });
    res.write("hello-");
    res.end("http2");
  };
}

const h2cResult = await roundTrip(createServer(makeHandler()), "http", undefined);
assert.equal(h2cResult.status, 201);
assert.equal(h2cResult.customHeader, "ok");
assert.equal(h2cResult.body, "hello-http2");

const h2TlsResult = await roundTrip(
  createSecureServer({ key, cert }, makeHandler()),
  "https",
  { ca, servername: "localhost" }
);
assert.equal(h2TlsResult.status, 201);
assert.equal(h2TlsResult.customHeader, "ok");
assert.equal(h2TlsResult.body, "hello-http2");

console.log("http2-server compat OK");
