// Fixture: a timer registered from another timer's continuation, while an
// HTTP keep-alive connection stays open. Must reach "second-timer" and exit.
// Top-level await is essential: it makes poll_timers drain the promise
// continuation itself, which is the only shape that exposes a timer list
// snapshotted before the callbacks ran.
import http from "node:http";
import net from "node:net";

const server = http.createServer((_req, res) => {
  res.writeHead(200);
  res.end("ok");
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));

const socket = net.connect(server.address().port, "127.0.0.1");
await new Promise((r) => socket.on("connect", r));
socket.write("GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n");
await new Promise((r) => socket.on("data", () => r()));
console.log("response");

await new Promise((r) => setTimeout(r, 150));
console.log("first-timer");

await new Promise((r) => setTimeout(r, 400));
console.log("second-timer");

socket.destroy();
server.close();
