import assert from "node:assert/strict";
import http from "node:http";
import net from "node:net";
import { once } from "node:events";

for (const insecureHTTPParser of [false, true]) {
  const retained = [];
  const server = http.createServer({ insecureHTTPParser }, (req, res) => {
    retained.push(res);
    res.end("ok");
  });
  server.keepAliveTimeout = 100;
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const socket = net.connect(server.address().port, "127.0.0.1");
  const deadline = setTimeout(() => {
    console.error("Expired keep-alive connection remained open");
    process.exit(1);
  }, 3000);
  try {
    const ended = once(socket, "end");
    const chunks = [];
    socket.on("data", (chunk) => chunks.push(chunk));
    await once(socket, "connect");
    socket.write("GET / HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n");
    await ended;
    assert.match(Buffer.concat(chunks).toString(), /\r\n\r\nok$/);
    assert.equal(retained.length, 1);
    console.log(`keepalive-retained=${insecureHTTPParser}:closed`);
  } finally {
    clearTimeout(deadline);
    socket.destroy();
    await new Promise((resolve) => server.close(resolve));
  }
}
